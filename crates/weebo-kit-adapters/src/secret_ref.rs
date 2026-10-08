use k8s_openapi::api::core::v1::Secret;
use kube::{Api, Client};
use weebo_kit_api::SecretKeyRef;

/// Bytes of one key of a `Secret`. `Ok(None)` when the Secret or the key
/// doesn't exist; `Err` for any other API failure.
pub async fn read_secret_key(
    client: &Client,
    secret_ref: &SecretKeyRef,
) -> Result<Option<Vec<u8>>, String> {
    let api: Api<Secret> = Api::namespaced(client.clone(), &secret_ref.namespace);
    let secret = api.get_opt(&secret_ref.name).await.map_err(|e| {
        format!(
            "reading secret {}/{}: {e}",
            secret_ref.namespace, secret_ref.name
        )
    })?;
    Ok(secret
        .and_then(|s| s.data)
        .and_then(|mut data| data.remove(&secret_ref.key))
        .map(|bytes| bytes.0))
}
