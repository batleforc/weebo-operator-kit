//! A `{{product}}Instance` has no remote object: reconciling it is a
//! connectivity and token check, whose result every CR bound to it
//! implicitly depends on.

use crate::ports::RemoteGateway;
use crate::use_cases::Outcome;

pub async fn reconcile_instance(gateway: &dyn RemoteGateway) -> Outcome {
    match gateway.ping().await {
        Ok(()) => Outcome::synced(None, "{{product}} answers"),
        Err(e) => Outcome::failed(e),
    }
}

#[cfg(test)]
mod tests {
    use domain::error::ReasonCode;

    use super::*;
    use crate::ports::GatewayError;
    use crate::test_support::FakeGateway;

    #[tokio::test]
    async fn a_reachable_instance_is_ready() {
        let outcome = reconcile_instance(&FakeGateway::default()).await;
        assert_eq!(outcome.reason(), ReasonCode::Reconciled);
        assert_eq!(outcome.remote_id, None);
    }

    #[tokio::test]
    async fn an_unreachable_or_refusing_instance_is_not() {
        let gateway = FakeGateway::default();
        gateway.fail_with(Some(GatewayError::Unreachable("dns".into())));
        assert_eq!(
            reconcile_instance(&gateway).await.reason(),
            ReasonCode::InstanceUnreachable
        );
        gateway.fail_with(Some(GatewayError::Unauthorized("401".into())));
        assert_eq!(
            reconcile_instance(&gateway).await.reason(),
            ReasonCode::InstanceUnauthorized
        );
    }
}
