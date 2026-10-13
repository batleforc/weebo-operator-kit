use k8s_openapi::api::core::v1::Secret;
use kube::{Api, Client};
use weebo_kit_api::{Annotations, SecretKeyRef};

/// Bytes of one key of a `Secret`. `Ok(None)` when the Secret or the key
/// doesn't exist; `Err` for any other API failure.
pub async fn read_secret_key(
    client: &Client,
    secret_ref: &SecretKeyRef,
) -> Result<Option<Vec<u8>>, String> {
    Ok(read_secret(client, secret_ref)
        .await?
        .and_then(|s| key_of(s, &secret_ref.key)))
}

/// Why a local instance's token can't be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TokenSecretError {
    /// The Secret exists but doesn't carry the consent label
    /// (`<domain>/instance-token: "true"`).
    #[error(
        "secret {secret} is not labelled {label}=\"true\": a local instance may only use a \
         Secret labelled for it as its token"
    )]
    NotAllowed { secret: String, label: String },
    /// Reading the Secret failed — usually transient.
    #[error("{0}")]
    Kubernetes(String),
}

/// [`read_secret_key`] for the token of a **local** instance (namespaced,
/// created by a namespace's users): the Secret must carry
/// `<domain>/instance-token: "true"`
/// ([`Annotations::instance_token_allowed`]). The operator reads Secrets
/// with its own rights: without that consent, anyone allowed to create a
/// local instance could send any Secret of the namespace to a URL of their
/// choosing. Cluster-scoped instances, created by cluster admins, keep
/// [`read_secret_key`]. `Ok(None)` when the Secret or the key doesn't
/// exist.
pub async fn read_local_instance_token(
    client: &Client,
    secret_ref: &SecretKeyRef,
    annotations: &Annotations,
) -> Result<Option<Vec<u8>>, TokenSecretError> {
    let Some(secret) = read_secret(client, secret_ref)
        .await
        .map_err(TokenSecretError::Kubernetes)?
    else {
        return Ok(None);
    };
    check_instance_token(&secret, secret_ref, annotations)?;
    Ok(key_of(secret, &secret_ref.key))
}

fn check_instance_token(
    secret: &Secret,
    secret_ref: &SecretKeyRef,
    annotations: &Annotations,
) -> Result<(), TokenSecretError> {
    if annotations.instance_token_allowed(&secret.metadata) {
        return Ok(());
    }
    Err(TokenSecretError::NotAllowed {
        secret: format!("{}/{}", secret_ref.namespace, secret_ref.name),
        label: annotations.key(weebo_kit_api::annotations::INSTANCE_TOKEN),
    })
}

async fn read_secret(client: &Client, secret_ref: &SecretKeyRef) -> Result<Option<Secret>, String> {
    let api: Api<Secret> = Api::namespaced(client.clone(), &secret_ref.namespace);
    api.get_opt(&secret_ref.name).await.map_err(|e| {
        format!(
            "reading secret {}/{}: {e}",
            secret_ref.namespace, secret_ref.name
        )
    })
}

fn key_of(secret: Secret, key: &str) -> Option<Vec<u8>> {
    secret
        .data
        .and_then(|mut data| data.remove(key))
        .map(|bytes| bytes.0)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    use super::*;

    const A: Annotations = Annotations::new("example.weebo.io");

    #[test]
    fn a_local_instance_token_needs_the_label() {
        let r = SecretKeyRef {
            name: "token".into(),
            namespace: "team-a".into(),
            key: "token".into(),
        };
        let mut secret = Secret::default();
        let err = check_instance_token(&secret, &r, &A).unwrap_err();
        assert_eq!(
            err,
            TokenSecretError::NotAllowed {
                secret: "team-a/token".into(),
                label: "example.weebo.io/instance-token".into(),
            }
        );
        secret.metadata = ObjectMeta {
            labels: Some(BTreeMap::from([(
                "example.weebo.io/instance-token".to_string(),
                "true".to_string(),
            )])),
            ..Default::default()
        };
        assert!(check_instance_token(&secret, &r, &A).is_ok());
    }
}
