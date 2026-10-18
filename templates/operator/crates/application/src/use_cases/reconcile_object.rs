//! A `{{product}}Object`: the remote object is read, compared with the
//! spec and written only when it differs — created when missing, updated
//! with the drifted fields only, left alone when in sync (a remote system
//! records every write: rewriting an unchanged object on every requeue
//! floods it for nothing). Deleted by the finalizer when this CR created
//! it.
//!
//! Compare to the form {{product}} **stores**: a server that trims a text
//! or completes a map with defaults needs the same normalization in the
//! comparison (`domain::drift::same_trimmed`, `same_subset`, through
//! `Drift::keep_with`), or the field drifts on every reconcile. Each such
//! normalization gets its test: the value read back normalized, no write.

use api::Object;
use domain::{Drift, Failure, RemoteWrite};

use crate::ports::{GatewayError, ObjectPatch, RemoteGateway, RemoteObject};
use crate::use_cases::Outcome;

/// The desired remote state of `object`.
pub fn desired(object: &Object) -> RemoteObject {
    RemoteObject {
        name: object.remote_name(),
        description: object.spec.description.clone(),
    }
}

/// The update `remote` needs to reach `desired`: every field already
/// holding its desired value left out, the drifted ones named.
pub fn patch(desired: &RemoteObject, remote: &RemoteObject) -> (ObjectPatch, Drift) {
    let mut patch = ObjectPatch {
        description: desired.description.clone(),
    };
    let mut drift = Drift::default();
    drift.keep_opt("description", &mut patch.description, &remote.description);
    (patch, drift)
}

pub async fn reconcile_object(object: &Object, gateway: &dyn RemoteGateway) -> Outcome {
    let desired = desired(object);
    let remote = match gateway.read(&desired.name).await {
        Ok(remote) => remote,
        Err(e) => return Outcome::failed(e),
    };
    let Some(remote) = remote else {
        return match gateway.create(&desired).await {
            Ok(id) => Outcome::synced(Some(id), format!("{:?} created", desired.name))
                .wrote(RemoteWrite::Created),
            Err(e) => Outcome::failed(e),
        };
    };
    let (patch, drift) = patch(&desired, &remote.object);
    if drift.is_empty() {
        return Outcome::synced(Some(remote.id), format!("{:?} in sync", desired.name));
    }
    match gateway.update(&desired.name, &patch).await {
        Ok(()) => {
            Outcome::synced(Some(remote.id), format!("{:?} synced", desired.name)).corrected(drift)
        }
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
    async fn creates_then_stays_quiet_until_drift() {
        let gateway = FakeGateway::default();
        let first = reconcile_object(&object("docs"), &gateway).await;
        assert_eq!(first.reason(), ReasonCode::Reconciled);
        assert_eq!(first.write, Some(RemoteWrite::Created));
        let id = first.remote_id.clone().expect("an id is recorded");
        assert_eq!(gateway.writes(), ["create docs"]);

        // Nothing changed: reads only.
        gateway.clear_writes();
        let quiet = reconcile_object(&object("docs"), &gateway).await;
        assert_eq!(quiet.remote_id, Some(id.clone()));
        assert_eq!((quiet.write, quiet.drift.is_empty()), (None, true));
        assert!(gateway.writes().is_empty(), "{:?}", gateway.writes());

        // Changed on the remote side: put back, that field only.
        gateway.edit("docs", |o| o.description = Some("by hand".into()));
        let corrected = reconcile_object(&object("docs"), &gateway).await;
        assert_eq!(corrected.write, Some(RemoteWrite::Drift));
        assert_eq!(corrected.drift.fields(), ["description"]);
        assert_eq!(corrected.remote_id, Some(id));
        assert_eq!(gateway.writes(), ["update docs [description]"]);
        assert_eq!(
            gateway.get("docs").unwrap().description.as_deref(),
            Some("example")
        );
    }

    #[tokio::test]
    async fn a_field_the_spec_leaves_unset_is_never_written() {
        let gateway = FakeGateway::default();
        reconcile_object(&object("docs"), &gateway).await;
        gateway.clear_writes();
        let mut unset = object("docs");
        unset.spec.description = None;
        let outcome = reconcile_object(&unset, &gateway).await;
        assert_eq!(outcome.write, None);
        assert!(gateway.writes().is_empty());
        assert_eq!(
            gateway.get("docs").unwrap().description.as_deref(),
            Some("example")
        );
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
