//! A `{{product}}Object`: the remote object is created or updated on every
//! reconcile (the gateway's `ensure` is idempotent), and deleted by the
//! finalizer when this CR created it.

use api::Object;
use domain::Failure;

use crate::ports::{GatewayError, RemoteGateway, RemoteObject};
use crate::use_cases::Outcome;

/// The desired remote state of `object`.
pub fn desired(object: &Object) -> RemoteObject {
    RemoteObject {
        name: object.remote_name(),
        description: object.spec.description.clone(),
    }
}

pub async fn reconcile_object(object: &Object, gateway: &dyn RemoteGateway) -> Outcome {
    let desired = desired(object);
    match gateway.ensure(&desired).await {
        Ok(id) => Outcome::synced(Some(id), format!("{:?} synced", desired.name)),
        Err(e) => Outcome::failed(e),
    }
}

/// The finalizer's cleanup. `remote_id`: the CR's `status.remoteId` — a CR
/// that never created anything deletes nothing (the object of that name
/// may belong to someone else). Already gone counts as deleted. Returns
/// whether something was deleted.
pub async fn delete_object(
    object: &Object,
    remote_id: Option<&str>,
    gateway: &dyn RemoteGateway,
) -> Result<bool, Failure> {
    if remote_id.is_none() {
        return Ok(false);
    }
    match gateway.delete(&object.remote_name()).await {
        Ok(()) | Err(GatewayError::NotFound(_)) => Ok(true),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use api::object::ObjectSpec;
    use domain::error::ReasonCode;

    use super::*;
    use crate::test_support::FakeGateway;

    fn object(name: &str) -> Object {
        Object::new(
            name,
            ObjectSpec {
                instance_ref: Some(api::object::InstanceRef::named("main")),
                name: None,
                description: Some("example".into()),
            },
        )
    }

    #[tokio::test]
    async fn reconcile_creates_then_updates_in_place() {
        let gateway = FakeGateway::default();
        let first = reconcile_object(&object("docs"), &gateway).await;
        assert_eq!(first.reason(), ReasonCode::Reconciled);
        let id = first.remote_id.clone().expect("an id is recorded");
        assert_eq!(
            gateway.get("docs").and_then(|o| o.description),
            Some("example".into())
        );

        let mut changed = object("docs");
        changed.spec.description = None;
        let second = reconcile_object(&changed, &gateway).await;
        assert_eq!(second.remote_id, Some(id));
        assert_eq!(gateway.get("docs").unwrap().description, None);
    }

    #[tokio::test]
    async fn a_failed_call_maps_to_its_reason() {
        let gateway = FakeGateway::default();
        gateway.fail_with(Some(GatewayError::Api("500".into())));
        let outcome = reconcile_object(&object("docs"), &gateway).await;
        assert_eq!(outcome.reason(), ReasonCode::RemoteError);
        assert_eq!(outcome.remote_id, None);
    }

    #[tokio::test]
    async fn cleanup_only_deletes_what_the_cr_created() {
        let gateway = FakeGateway::default();
        reconcile_object(&object("docs"), &gateway).await;
        assert_eq!(
            delete_object(&object("docs"), None, &gateway).await,
            Ok(false)
        );
        assert!(gateway.get("docs").is_some());
        assert_eq!(
            delete_object(&object("docs"), Some("1"), &gateway).await,
            Ok(true)
        );
        assert!(gateway.get("docs").is_none());
        // Already gone: nothing left to do.
        assert_eq!(
            delete_object(&object("docs"), Some("1"), &gateway).await,
            Ok(true)
        );
    }

    #[tokio::test]
    async fn a_failed_cleanup_keeps_the_finalizer() {
        let gateway = FakeGateway::default();
        gateway.fail_with(Some(GatewayError::Unreachable("timeout".into())));
        let err = delete_object(&object("docs"), Some("1"), &gateway)
            .await
            .unwrap_err();
        assert_eq!(err.reason, ReasonCode::InstanceUnreachable);
    }
}
