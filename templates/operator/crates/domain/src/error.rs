//! Single source of truth for every reason the operator surfaces. Status
//! conditions, Kubernetes Events and metrics all carry one of these
//! variants — never an ad-hoc string at the call site. Add a variant here
//! before reporting a new cause anywhere.

pub use weebo_kit_domain::{Reason, Severity};

weebo_kit_domain::reason_codes! {
    pub enum ReasonCode {
        /// `Ready: True` — the one success reason every reconciler uses.
        Reconciled,
        /// `spec.instanceRef` names no `{{product}}Instance` nor
        /// `{{product}}LocalInstance` the operator sees (or, unset, there is
        /// none to default to).
        InstanceNotFound,
        /// `spec.instanceRef` is unset and several instances could serve
        /// the object.
        InstanceAmbiguous,
        /// The instance named is out of reach: a `{{product}}LocalInstance`
        /// of another namespace that doesn't share itself with this one, or
        /// outside the namespace the operator is limited to.
        InstanceNotAllowed,
        /// `spec.instanceRef` by name now designates another instance than
        /// the one recorded in `status.instance`: not followed until
        /// `spec.instanceRef.kind` says which.
        InstanceBindingChanged,
        /// The instance's `tokenSecretRef` Secret, or its key, is missing
        /// or not UTF-8.
        TokenSecretMissing,
        /// The instance's spec can't produce a client (e.g. an invalid
        /// `url`). Only a change of the CR clears it.
        InvalidSpec,
        /// {{product}} did not answer (network error, timeout, 502/503/504).
        InstanceUnreachable,
        /// {{product}} refused the instance's token (401/403).
        InstanceUnauthorized,
        /// {{product}} answered with an unexpected error.
        RemoteError,
        /// A Kubernetes API call of the operator itself failed (reading a
        /// CR or a Secret, patching a status, the finalizer) — retried.
        KubernetesApiError,
        /// The remote object was created for this CR (Event only).
        Created,
        /// The finalizer deleted the remote object of this CR (Event only).
        Deleted,
        /// The finalizer finished — cleanup done, or the CR force-orphaned
        /// (Event only).
        Released,
        /// An update would take a CR still carrying the finalizer out of
        /// the operator's label selector (`watch.labelSelector`): refused
        /// at admission.
        LeavesWatchScope,
    }
}

impl Reason for ReasonCode {
    const SUCCESS: Self = ReasonCode::Reconciled;

    fn as_str(self) -> &'static str {
        ReasonCode::as_str(self)
    }

    fn severity(self) -> Severity {
        ReasonCode::severity(self)
    }
}

impl ReasonCode {
    /// Advisory codes don't block `Ready` (and count as `success` in the
    /// reconcile metrics); every failure cause is blocking.
    pub const fn severity(self) -> Severity {
        match self {
            ReasonCode::Reconciled
            | ReasonCode::Created
            | ReasonCode::Deleted
            | ReasonCode::Released => Severity::Advisory,
            _ => Severity::Blocking,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_is_pascal_case() {
        weebo_kit_domain::reason::assert_pascal_case(ReasonCode::ALL);
    }

    #[test]
    fn only_success_and_lifecycle_events_are_advisory() {
        let advisory: Vec<_> = ReasonCode::ALL
            .iter()
            .filter(|c| c.severity() == Severity::Advisory)
            .collect();
        assert_eq!(
            advisory,
            [
                &ReasonCode::Reconciled,
                &ReasonCode::Created,
                &ReasonCode::Deleted,
                &ReasonCode::Released
            ]
        );
    }
}
